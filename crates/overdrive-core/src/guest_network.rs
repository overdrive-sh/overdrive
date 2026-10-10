//! Shared guest-network admission capabilities.
//!
//! RED-ready API scaffold for GH #295. The public surface is copied exactly
//! from the accepted DESIGN contract in
//! `docs/feature/netns-density-295/feature-delta.md` under
//! "EXEC-close linearization". DELIVER replaces the scaffold bodies; DISTILL
//! intentionally supplies no production behaviour.

// SCAFFOLD: true — netns-density-295 DISTILL.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::Clock;
use parking_lot::Mutex;
use tokio::sync::Notify;

/// Paired construction authority for the release and recovery capabilities.
pub struct GuestNetworkExecWiring {
    gate: Arc<GuestNetworkExecGate>,
    supervisor: Arc<GuestNetworkExecSupervisor>,
}

/// Read/claim capability retained by the VM driver.
pub struct GuestNetworkExecGate {
    shared: Arc<GuestNetworkExecShared>,
}

/// Recovery/reopen/fail-stop capability retained by the control plane.
pub struct GuestNetworkExecSupervisor {
    shared: Arc<GuestNetworkExecShared>,
}

/// RAII claim held across one deferred-EXEC writer acknowledgement.
#[must_use]
pub struct GuestNetworkExecClaim {
    shared: Arc<GuestNetworkExecShared>,
}

struct GuestNetworkExecShared {
    state: Mutex<GuestNetworkExecState>,
    notify: Notify,
    clock: Arc<dyn Clock>,
}

struct GuestNetworkExecState {
    state: GuestNetworkExecGateState,
    active_claims: usize,
}

struct RecoverySnapshot {
    component: SharedGuestNetworkComponent,
    started_at: Instant,
    completed_attempts: u32,
}

enum GuestNetworkExecGateState {
    BootClosed,
    Open,
    Recovering(RecoverySnapshot),
    FailStop,
}

/// Closed vocabulary for the shared guest-network components the supervisor
/// audits and repairs (feature-delta § *Typed error taxonomy*). Declaration
/// order is the fixed audit order. Derives `Ord` (§ *Owner and provisioner*,
/// D8a-HOLD): the supervisor's per-holder state and `QuiescenceHolder` order
/// by it. The pinned guest-prefix steering, the prefix route and the
/// forwarder's links are not components (A-31, D19-LINKS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SharedGuestNetworkComponent {
    /// The forwarder's pair and cell bookkeeping: the owner's pair table
    /// against the forwarder's inventory.
    Forwarder,
    /// The host guest-flow vsock listeners (stream and seqpacket).
    FlowListeners,
    /// The host per-VM control-session vsock listener.
    ControlListener,
    /// The per-allocation intake listeners.
    IntakeListeners,
    /// Workload-facing plaintext listener.
    LegF,
    /// Peer-facing inbound TLS listener.
    LegC,
    /// Constant IP rule program.
    IpRules,
    /// Shared IP sets and their schemas.
    IpSets,
    /// Shared-gateway DNS owner.
    Dns,
    /// Shared-owner supervisor itself.
    Supervisor,
}

/// Closed vocabulary for terminal shared-network supervisor outcomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SharedGuestNetworkFailStopCause {
    /// The bounded recovery deadline elapsed.
    RecoveryDeadlineExceeded,
    /// The set of TAPs that failed to go down cannot be determined, so the
    /// whole workloads slice was killed (D-295-R14).
    ///
    /// RED scaffold (D-295-R14): constructed in DELIVER step 09-01.
    TapQuiescenceUndetermined,
    /// A per-VM kill write for a known allocation failed other than with an
    /// absent scope, so the whole workloads slice was killed (D-295-R14).
    ///
    /// RED scaffold (D-295-R14): constructed in DELIVER step 09-01.
    VmKillFailed,
    /// The supervisor returned without a fail-stop request.
    SupervisorReturned,
    /// The supervisor returned a typed failure.
    SupervisorFailed,
    /// The supervisor task panicked.
    SupervisorPanicked,
    /// The supervisor task was cancelled unexpectedly.
    SupervisorCancelled,
    /// The fail-stop request channel closed unexpectedly.
    RequestChannelClosed,
}

/// Read-only recovery progress of one recovery holder while the EXEC gate is
/// recovering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedGuestNetworkRecovery {
    /// The component under repair.
    pub component: SharedGuestNetworkComponent,
    /// Completed converge-plus-full-read-back attempts.
    pub attempts: u32,
    /// Monotonic time elapsed since detection.
    pub elapsed: Duration,
}

/// Public fail-stop receipt sent to the CLI-owned serve lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SharedGuestNetworkFailStop {
    /// Latest failing component, or [`SharedGuestNetworkComponent::Supervisor`].
    pub component: SharedGuestNetworkComponent,
    /// Terminal cause observed by the sole supervisor owner.
    pub cause: SharedGuestNetworkFailStopCause,
    /// Completed recovery attempts.
    pub attempts: u32,
    /// Monotonic time elapsed since detection.
    pub elapsed: Duration,
}

/// Internal request crossing from the server handle to the CLI lifetime.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServeShutdownRequest {
    /// Shared guest-network ownership could not be recovered.
    SharedGuestNetwork(SharedGuestNetworkFailStop),
}

/// Fixed placeholder cap on held guest-network attachments per node. It has no
/// capacity basis and promises no density; real per-node capacity is GH #299
/// and GH #261. Enforced at ADR-0132's linearization point.
pub const MAX_GUEST_NETWORK_ATTACHMENTS: u32 = 16_384;

/// One consistent reading of node guest-attachment occupancy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GuestAttachmentOccupancy {
    /// Admitted plus Retiring leases (ADR-0133). Compared against the cap.
    pub held: u32,
    /// The Retiring subset of `held`: attachments whose cleanup has begun and
    /// not finished. Reported so a stuck cleanup is visible.
    pub retiring: u32,
}

impl GuestNetworkExecWiring {
    /// Construct one BootClosed state and its paired opaque capabilities.
    /// CONTRACT_SHAPE: pure-function.
    #[must_use]
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        let shared = Arc::new(GuestNetworkExecShared {
            state: Mutex::new(GuestNetworkExecState {
                state: GuestNetworkExecGateState::BootClosed,
                active_claims: 0,
            }),
            notify: Notify::new(),
            clock,
        });
        Self {
            gate: Arc::new(GuestNetworkExecGate { shared: Arc::clone(&shared) }),
            supervisor: Arc::new(GuestNetworkExecSupervisor { shared }),
        }
    }

    /// Return the VM driver's claim-only capability.
    #[must_use]
    pub fn gate(&self) -> Arc<GuestNetworkExecGate> {
        Arc::clone(&self.gate)
    }

    /// Return the control-plane recovery capability.
    #[must_use]
    pub fn supervisor(&self) -> Arc<GuestNetworkExecSupervisor> {
        Arc::clone(&self.supervisor)
    }
}

impl GuestNetworkExecGate {
    /// Wait while closed/recovering, claim when open, or return `None` at fail-stop.
    #[must_use]
    pub async fn claim_release(&self) -> Option<GuestNetworkExecClaim> {
        loop {
            let mut notified = Box::pin(self.shared.notify.notified());
            let claim = {
                let mut shared = self.shared.state.lock();
                match &shared.state {
                    GuestNetworkExecGateState::Open => {
                        shared.active_claims = shared.active_claims.saturating_add(1);
                        drop(shared);
                        Some(GuestNetworkExecClaim { shared: Arc::clone(&self.shared) })
                    }
                    GuestNetworkExecGateState::FailStop => return None,
                    GuestNetworkExecGateState::BootClosed
                    | GuestNetworkExecGateState::Recovering(_) => {
                        notified.as_mut().enable();
                        None
                    }
                }
            };
            if claim.is_some() {
                return claim;
            }
            notified.await;
        }
    }
}

impl GuestNetworkExecSupervisor {
    /// Report whether fresh-process target recovery may mutate owned targets.
    #[must_use]
    pub fn is_boot_closed(&self) -> bool {
        matches!(self.shared.state.lock().state, GuestNetworkExecGateState::BootClosed)
    }

    /// Perform the sole BootClosed-to-Open transition.
    #[must_use]
    pub fn open_after_boot(&self) -> bool {
        let opened = {
            let mut shared = self.shared.state.lock();
            if matches!(shared.state, GuestNetworkExecGateState::BootClosed) {
                shared.state = GuestNetworkExecGateState::Open;
                true
            } else {
                false
            }
        };
        if opened {
            self.shared.notify.notify_waiters();
        }
        opened
    }

    /// Perform the sole Open-to-Recovering transition.
    #[must_use]
    pub fn begin_recovery(&self, component: SharedGuestNetworkComponent) -> bool {
        let began = {
            let mut shared = self.shared.state.lock();
            if matches!(shared.state, GuestNetworkExecGateState::Open) {
                shared.state = GuestNetworkExecGateState::Recovering(RecoverySnapshot {
                    component,
                    started_at: self.shared.clock.now(),
                    completed_attempts: 0,
                });
                true
            } else {
                false
            }
        };
        if began {
            self.shared.notify.notify_waiters();
        }
        began
    }

    /// Record one completed attempt and either remain recovering or reopen.
    /// `remaining` is the set of components still failing after the
    /// attempt's full audit; an empty set reopens the gate.
    ///
    /// Today's single-holder bookkeeping is kept: the first member of
    /// `remaining` in `Ord` becomes the recovery snapshot's component. The
    /// per-holder contract (§ *Owner and provisioner*, U-QP-3) lands with
    /// the DELIVER 09-01 rework in the attachment cut.
    #[must_use]
    #[allow(
        clippy::needless_pass_by_value,
        reason = "the EXEC gate contract pins `remaining` by value; the per-holder bookkeeping consumes it"
    )]
    pub fn complete_attempt(&self, remaining: BTreeSet<SharedGuestNetworkComponent>) -> bool {
        let first_remaining = remaining.first().copied();
        let completed = {
            let mut shared = self.shared.state.lock();
            let GuestNetworkExecGateState::Recovering(snapshot) = &mut shared.state else {
                return false;
            };
            let attempts = snapshot.completed_attempts.saturating_add(1);
            match first_remaining {
                Some(component) => {
                    snapshot.component = component;
                    snapshot.completed_attempts = attempts;
                }
                None => shared.state = GuestNetworkExecGateState::Open,
            }
            true
        };
        if completed {
            self.shared.notify.notify_waiters();
        }
        completed
    }

    /// Report the current recovery holders: one entry per component under
    /// repair, ascending by [`SharedGuestNetworkComponent`]'s `Ord`; empty in
    /// every state except Recovering. A value snapshot taken under the gate
    /// lock (U-QP-3).
    ///
    /// Today's single-holder bookkeeping yields at most one entry; the
    /// per-holder set (D8a-HOLD) lands with the DELIVER 09-01 rework in the
    /// attachment cut.
    #[must_use]
    pub fn recovery_progress(&self) -> Vec<SharedGuestNetworkRecovery> {
        let shared = self.shared.state.lock();
        let GuestNetworkExecGateState::Recovering(snapshot) = &shared.state else {
            return Vec::new();
        };
        let progress = SharedGuestNetworkRecovery {
            component: snapshot.component,
            attempts: snapshot.completed_attempts,
            elapsed: self.shared.clock.now().saturating_duration_since(snapshot.started_at),
        };
        drop(shared);
        vec![progress]
    }

    /// Enter FailStop once and return the first public request receipt.
    /// CONTRACT_SHAPE: bounded-change.
    #[must_use]
    pub fn fail_stop(
        &self,
        cause: SharedGuestNetworkFailStopCause,
    ) -> Option<SharedGuestNetworkFailStop> {
        let request = {
            let mut shared = self.shared.state.lock();
            let (component, attempts, elapsed) = match &shared.state {
                GuestNetworkExecGateState::FailStop => return None,
                GuestNetworkExecGateState::Recovering(snapshot) => (
                    snapshot.component,
                    snapshot.completed_attempts,
                    self.shared.clock.now().saturating_duration_since(snapshot.started_at),
                ),
                GuestNetworkExecGateState::BootClosed | GuestNetworkExecGateState::Open => {
                    (SharedGuestNetworkComponent::Supervisor, 0, Duration::ZERO)
                }
            };
            shared.state = GuestNetworkExecGateState::FailStop;
            let request = SharedGuestNetworkFailStop { component, cause, attempts, elapsed };
            drop(shared);
            request
        };
        self.shared.notify.notify_waiters();
        Some(request)
    }
}

impl Drop for GuestNetworkExecClaim {
    fn drop(&mut self) {
        let mut shared = self.shared.state.lock();
        debug_assert!(shared.active_claims > 0, "an EXEC claim must be accounted before Drop");
        shared.active_claims = shared.active_claims.saturating_sub(1);
        drop(shared);
        self.shared.notify.notify_waiters();
    }
}
